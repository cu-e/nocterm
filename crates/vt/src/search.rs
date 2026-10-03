//! Incremental literal search over the retained grid, with O(query) memory.
use std::collections::VecDeque;

use alacritty_terminal::{
    grid::Dimensions as _,
    index::{Column, Line},
    term::cell::Flags,
};

use crate::Emulator;

/// Maximum query length in Unicode scalar values.
pub const MAX_SEARCH_QUERY: usize = 2_048;

/// A text position, including the offset of a combining character on its cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SearchPoint {
    pub line: i32,
    pub column: u16,
    pub(crate) part: usize,
}

/// Inclusive cell range. Search never replaces the user's selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: SearchPoint,
    pub end: SearchPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDirection {
    Next,
    Previous,
    Stay,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchResult {
    pub active: Option<SearchMatch>,
    pub ordinal: u64,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProgress {
    Searching,
    Invalidated,
    Complete(SearchResult),
}

/// One cancellable scan. No grid or list of all matches is copied.
pub struct SearchScan {
    generation: u64,
    query: Vec<char>,
    prefix: Vec<usize>,
    ring: VecDeque<SearchPoint>,
    matched: usize,
    line: i32,
    end_line: i32,
    column: usize,
    part: usize,
    trim: Option<usize>,
    end_column: usize,
    wrapped: bool,
    anchor: Option<SearchPoint>,
    direction: SearchDirection,
    first: Option<SearchMatch>,
    last: Option<SearchMatch>,
    best: Option<(SearchMatch, u64)>,
    count: u64,
    complete: Option<SearchResult>,
}

impl SearchScan {
    /// The first eligible forward match is already final before counting ends.
    /// Previous and wrap-around searches must finish the scan first.
    pub fn provisional(&self) -> Option<SearchMatch> {
        (self.direction != SearchDirection::Previous)
            .then_some(self.best)
            .flatten()
            .map(|(found, _)| found)
    }
    pub(crate) fn new(
        emulator: &Emulator,
        query: &str,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
    ) -> Result<Self, String> {
        let query: Vec<_> = query.chars().take(MAX_SEARCH_QUERY + 1).collect();
        if query.len() > MAX_SEARCH_QUERY {
            return Err(format!(
                "Search is limited to {MAX_SEARCH_QUERY} characters."
            ));
        }
        if query.is_empty() {
            return Err("Enter text to search.".into());
        }
        let mut prefix = vec![0; query.len()];
        for i in 1..query.len() {
            let mut j = prefix[i - 1];
            while j > 0 && query[i] != query[j] {
                j = prefix[j - 1];
            }
            if query[i] == query[j] {
                j += 1;
            }
            prefix[i] = j;
        }
        Ok(Self {
            generation: emulator.generation(),
            query,
            prefix,
            ring: VecDeque::new(),
            matched: 0,
            line: emulator.search_grid().topmost_line().0,
            end_line: emulator.search_grid().bottommost_line().0,
            column: 0,
            part: 0,
            trim: None,
            end_column: 0,
            wrapped: false,
            anchor,
            direction,
            first: None,
            last: None,
            best: None,
            count: 0,
            complete: None,
        })
    }

    /// At most `budget` cell inspections or Unicode scalars are visited.
    pub fn step(&mut self, emulator: &Emulator, budget: usize) -> SearchProgress {
        if self.generation != emulator.generation() {
            return SearchProgress::Invalidated;
        }
        if let Some(result) = self.complete {
            return SearchProgress::Complete(result);
        }
        let grid = emulator.search_grid();
        for _ in 0..budget {
            if self.line > self.end_line {
                let chosen = self.best.or_else(|| match self.direction {
                    SearchDirection::Previous => self.last.map(|m| (m, self.count)),
                    _ => self.first.map(|m| (m, 1)),
                });
                let result = SearchResult {
                    active: chosen.map(|(m, _)| m),
                    ordinal: chosen.map_or(0, |(_, n)| n),
                    count: self.count,
                };
                self.complete = Some(result);
                return SearchProgress::Complete(result);
            }
            if self.trim.is_none() {
                self.wrapped = grid[Line(self.line)][Column(grid.columns() - 1)]
                    .flags
                    .contains(Flags::WRAPLINE);
                self.trim = Some(grid.columns());
            }
            if self.column == 0 && self.part == 0 {
                let trim = self.trim.unwrap();
                if !self.wrapped && trim > 0 {
                    let cell = &grid[Line(self.line)][Column(trim - 1)];
                    if cell.c == ' ' && cell.zerowidth().is_none_or(|s| s.is_empty()) {
                        self.trim = Some(trim - 1);
                        continue;
                    }
                }
                self.end_column = trim;
            }
            if self.column >= self.end_column {
                if !self.wrapped {
                    self.consume(
                        '\n',
                        SearchPoint {
                            line: self.line,
                            column: self.end_column.min(grid.columns() - 1) as u16,
                            part: usize::MAX,
                        },
                    );
                }
                self.line += 1;
                self.column = 0;
                self.part = 0;
                self.trim = None;
                continue;
            }
            let cell = &grid[Line(self.line)][Column(self.column)];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                self.column += 1;
                self.part = 0;
                continue;
            }
            let scalar = if self.part == 0 {
                Some(cell.c)
            } else {
                cell.zerowidth().and_then(|s| s.get(self.part - 1)).copied()
            };
            if let Some(scalar) = scalar {
                self.consume(
                    scalar,
                    SearchPoint {
                        line: self.line,
                        column: self.column as u16,
                        part: self.part,
                    },
                );
                self.part += 1;
            } else {
                self.column += 1;
                self.part = 0;
            }
        }
        SearchProgress::Searching
    }

    fn consume(&mut self, scalar: char, point: SearchPoint) {
        self.ring.push_back(point);
        if self.ring.len() > self.query.len() {
            self.ring.pop_front();
        }
        while self.matched > 0 && scalar != self.query[self.matched] {
            self.matched = self.prefix[self.matched - 1];
        }
        if scalar == self.query[self.matched] {
            self.matched += 1;
        }
        if self.matched != self.query.len() {
            return;
        }
        let found = SearchMatch {
            start: *self.ring.front().unwrap(),
            end: point,
        };
        self.count = self.count.saturating_add(1);
        self.first.get_or_insert(found);
        self.last = Some(found);
        let eligible = self.anchor.is_none_or(|anchor| match self.direction {
            SearchDirection::Next => found.start > anchor,
            SearchDirection::Previous => found.start < anchor,
            SearchDirection::Stay => found.start >= anchor,
        });
        if eligible && (self.best.is_none() || self.direction == SearchDirection::Previous) {
            self.best = Some((found, self.count));
        }
        self.matched = self.prefix[self.matched - 1];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EmulatorOptions, TermSize};
    fn finish(
        e: &Emulator,
        query: &str,
        direction: SearchDirection,
        anchor: Option<SearchPoint>,
    ) -> SearchResult {
        let mut scan = e.search(query, direction, anchor).unwrap();
        loop {
            if let SearchProgress::Complete(result) = scan.step(e, 7) {
                return result;
            }
        }
    }
    #[test]
    fn unicode_wide_combining_soft_wrap_and_hard_newline() {
        let mut e = Emulator::new(TermSize::new(6, 3, 0, 0), EmulatorOptions::default());
        e.advance("ab界e\u{301}fg\r\nlast".as_bytes());
        let result = finish(&e, "界e\u{301}fg", SearchDirection::Next, None);
        assert_eq!(result.count, 1);
        assert_ne!(
            result.active.unwrap().start.line,
            result.active.unwrap().end.line
        );
        assert_eq!(finish(&e, "fg\nlast", SearchDirection::Next, None).count, 1);
        assert_eq!(finish(&e, "fglast", SearchDirection::Next, None).count, 0);
    }
    #[test]
    fn overlaps_next_previous_wrap_and_no_match_limit() {
        let mut e = Emulator::new(TermSize::new(80, 5, 0, 0), EmulatorOptions::default());
        e.advance(format!("{}\r\n", "a".repeat(5_000)).as_bytes());
        let first = finish(&e, "aa", SearchDirection::Next, None);
        assert_eq!(first.count, 4_999);
        let second = finish(
            &e,
            "aa",
            SearchDirection::Next,
            first.active.map(|m| m.start),
        );
        assert_eq!(second.ordinal, 2);
        let last = finish(
            &e,
            "aa",
            SearchDirection::Previous,
            first.active.map(|m| m.start),
        );
        assert_eq!(last.ordinal, 4_999);
        let wrapped = finish(
            &e,
            "aa",
            SearchDirection::Next,
            last.active.map(|m| m.start),
        );
        assert_eq!(wrapped.active, first.active);
    }
    #[test]
    fn bounded_steps_invalidation_empty_and_query_limit() {
        let mut e = Emulator::new(TermSize::default(), EmulatorOptions::default());
        e.advance(b"one\r\ntwo");
        assert!(e.search("", SearchDirection::Next, None).is_err());
        assert!(
            e.search(
                &"a".repeat(MAX_SEARCH_QUERY + 1),
                SearchDirection::Next,
                None
            )
            .is_err()
        );
        let mut scan = e.search("two", SearchDirection::Next, None).unwrap();
        assert_eq!(scan.step(&e, 1), SearchProgress::Searching);
        e.advance(b"changed");
        assert_eq!(scan.step(&e, 1), SearchProgress::Invalidated);
    }

    #[test]
    fn search_highlight_never_changes_selection_and_history_eviction_invalidates() {
        use crate::{CellPoint, Frame, SelectionKind, Side};
        let mut e = Emulator::new(TermSize::new(12, 2, 0, 0), EmulatorOptions::default());
        e.advance(b"pick needle\r\nnext\r\nlast");
        let result = finish(&e, "needle", SearchDirection::Next, None);
        assert!(
            result.active.unwrap().start.line < 0,
            "retained history is searched"
        );
        e.show_search_match(result.active.unwrap());
        e.start_selection(
            SelectionKind::Cells,
            CellPoint { row: 0, col: 0 },
            Side::Left,
        );
        e.update_selection(CellPoint { row: 0, col: 3 }, Side::Right);
        assert_eq!(e.selection_text().as_deref(), Some("pick"));
        let mut frame = Frame::default();
        e.snapshot(&mut frame);
        assert_eq!(frame.cells.iter().filter(|c| c.selected).count(), 4);
        assert_eq!(frame.cells.iter().filter(|c| c.search_hit).count(), 6);
        e.clear_selection();
        e.snapshot(&mut frame);
        assert!(!frame.cells.iter().any(|c| c.selected));
        assert!(frame.cells.iter().any(|c| c.search_hit));
        let mut scan = e.search("needle", SearchDirection::Next, None).unwrap();
        e.set_options(EmulatorOptions {
            scrollback_lines: 0,
            ..Default::default()
        });
        assert_eq!(scan.step(&e, 1), SearchProgress::Invalidated);
        assert_eq!(finish(&e, "needle", SearchDirection::Next, None).count, 0);
    }

    #[test]
    fn resize_reflow_and_alternate_screen_do_not_keep_stale_matches() {
        use crate::Frame;
        let mut e = Emulator::new(TermSize::new(8, 4, 0, 0), EmulatorOptions::default());
        e.advance(b"literal[.*]needle");
        let result = finish(&e, "[.*]needle", SearchDirection::Next, None);
        assert_eq!(result.count, 1);
        e.show_search_match(result.active.unwrap());
        let mut scan = e.search("needle", SearchDirection::Next, None).unwrap();
        e.resize(TermSize::new(16, 4, 0, 0));
        assert_eq!(scan.step(&e, 1), SearchProgress::Invalidated);
        assert_eq!(
            finish(&e, "[.*]needle", SearchDirection::Next, None).count,
            1
        );
        let mut frame = Frame::default();
        e.snapshot(&mut frame);
        assert!(!frame.cells.iter().any(|c| c.search_hit));
        e.advance(b"\x1b[?1049halternate");
        assert_eq!(finish(&e, "needle", SearchDirection::Next, None).count, 0);
        e.advance(b"\x1b[?1049l");
        assert_eq!(finish(&e, "needle", SearchDirection::Next, None).count, 1);
    }

    #[test]
    fn many_combining_scalars_use_bounded_steps_and_distinct_overlap_anchors() {
        let mut e = Emulator::new(TermSize::new(100, 2, 0, 0), EmulatorOptions::default());
        // Keep thousands of scalars under tiny scheduler budgets while respecting
        // the engine's 64-mark cap per cell.
        let cluster = format!("a{}", "\u{301}".repeat(50));
        e.advance(cluster.repeat(100).as_bytes());
        let mut scan = e
            .search("\u{301}\u{301}", SearchDirection::Next, None)
            .unwrap();
        for _ in 0..100 {
            assert_eq!(scan.step(&e, 1), SearchProgress::Searching);
        }
        let first = finish(&e, "\u{301}\u{301}", SearchDirection::Next, None);
        assert_eq!(first.count, 100 * 49);
        let second = finish(
            &e,
            "\u{301}\u{301}",
            SearchDirection::Next,
            first.active.map(|m| m.start),
        );
        assert_eq!(second.ordinal, 2);
        assert_eq!(
            first.active.unwrap().start.column,
            second.active.unwrap().start.column
        );
        assert_ne!(first.active.unwrap().start, second.active.unwrap().start);
    }
}
