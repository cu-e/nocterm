use super::*;

impl Emulator {
    /// Moves the viewport through the scrollback, returning whether its display changed.
    pub fn scroll(&mut self, scroll: Scroll) -> bool {
        let offset = self.display_offset();
        let history = self.term.grid().history_size();
        let at_boundary = match scroll {
            Scroll::Lines(0) => true,
            Scroll::Lines(lines) if lines > 0 => offset == history,
            Scroll::Lines(_) => offset == 0,
            Scroll::PageUp | Scroll::Top => offset == history,
            Scroll::PageDown | Scroll::Bottom => offset == 0,
        };
        // Vi scrolling can also clamp its cursor and recompute selection.
        let vi_state = self
            .term
            .mode()
            .contains(TermMode::VI)
            .then(|| (self.term.vi_mode_cursor, self.term.selection.clone()));
        if at_boundary && vi_state.is_none() {
            return false;
        }
        self.term.scroll_display(match scroll {
            Scroll::Lines(lines) => GridScroll::Delta(lines),
            Scroll::PageUp => GridScroll::PageUp,
            Scroll::PageDown => GridScroll::PageDown,
            Scroll::Top => GridScroll::Top,
            Scroll::Bottom => GridScroll::Bottom,
        });
        offset != self.display_offset()
            || vi_state.is_some_and(|(cursor, selection)| {
                cursor != self.term.vi_mode_cursor || selection != self.term.selection
            })
    }

    /// Lines the viewport is scrolled back by; 0 shows the live screen.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Whether a selection anchor exists, including a drag with no cells selected yet.
    pub fn has_selection(&self) -> bool {
        self.term.selection.is_some()
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    /// Prepares typed input, returning whether selection or scrollback changed.
    pub fn prepare_input(&mut self) -> bool {
        let changed = self.display_offset() != 0 || self.term.selection.is_some();
        if changed {
            self.scroll(Scroll::Bottom);
            self.clear_selection();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emulator() -> Emulator {
        Emulator::new(TermSize::new(10, 3, 0, 0), EmulatorOptions::default())
    }

    fn frame(emulator: &Emulator) -> Frame {
        let mut frame = Frame::default();
        emulator.snapshot(&mut frame);
        frame
    }

    #[test]
    fn empty_history_scrolls_do_not_change_frame_or_queue_cursor_events() {
        let mut emulator = emulator();
        let before = frame(&emulator);
        for scroll in [
            Scroll::Top,
            Scroll::Bottom,
            Scroll::PageUp,
            Scroll::PageDown,
            Scroll::Lines(0),
            Scroll::Lines(10),
            Scroll::Lines(-10),
        ] {
            assert!(!emulator.scroll(scroll));
            assert_eq!(frame(&emulator), before);
        }
        assert!(emulator.events.lock().unwrap().is_empty());
    }

    #[test]
    fn history_boundary_and_zero_delta_scrolls_are_noops_but_pages_move() {
        let mut emulator = emulator();
        emulator.advance("line\r\n".repeat(10).as_bytes());
        for offset in [3, 6, 8] {
            assert!(emulator.scroll(Scroll::PageUp));
            assert_eq!(emulator.display_offset(), offset);
        }
        for scroll in [
            Scroll::PageUp,
            Scroll::Top,
            Scroll::Lines(1),
            Scroll::Lines(0),
        ] {
            assert!(!emulator.scroll(scroll));
            assert_eq!(emulator.display_offset(), 8);
        }
        for offset in [5, 2, 0] {
            assert!(emulator.scroll(Scroll::PageDown));
            assert_eq!(emulator.display_offset(), offset);
        }
        for scroll in [
            Scroll::PageDown,
            Scroll::Bottom,
            Scroll::Lines(-1),
            Scroll::Lines(0),
        ] {
            assert!(!emulator.scroll(scroll));
            assert_eq!(emulator.display_offset(), 0);
        }
        assert!(emulator.scroll(Scroll::Top));
        assert!(emulator.scroll(Scroll::Bottom));
    }

    #[test]
    fn boundary_scroll_keeps_selection_and_search_unchanged() {
        let mut emulator = emulator();
        emulator.advance("line\r\n".repeat(10).as_bytes());
        assert!(emulator.scroll(Scroll::Top));
        emulator.start_selection(
            SelectionKind::Cells,
            CellPoint { row: 0, col: 0 },
            Side::Left,
        );
        emulator.update_selection(CellPoint { row: 0, col: 2 }, Side::Right);
        emulator.show_search_match(crate::SearchMatch {
            start: crate::SearchPoint {
                line: -8,
                column: 0,
                part: 0,
            },
            end: crate::SearchPoint {
                line: -8,
                column: 2,
                part: 0,
            },
        });
        let selection = emulator.term.selection.clone();
        let search = emulator.search_match;
        let before = frame(&emulator);
        for scroll in [
            Scroll::Top,
            Scroll::PageUp,
            Scroll::Lines(1),
            Scroll::Lines(0),
        ] {
            assert!(!emulator.scroll(scroll));
            assert_eq!(emulator.term.selection, selection);
            assert_eq!(emulator.search_match, search);
            assert_eq!(frame(&emulator), before);
            assert_eq!(emulator.selection_text().as_deref(), Some("lin"));
        }
        assert!(emulator.scroll(Scroll::Bottom));
        assert_eq!(emulator.term.selection, selection);
        assert_eq!(emulator.search_match, search);
        assert_eq!(emulator.selection_text().as_deref(), Some("lin"));
    }

    #[test]
    fn vi_selection_recomputation_is_reported_even_without_viewport_movement() {
        let mut emulator = emulator();
        emulator.advance(b"abcdef");
        emulator.term.toggle_vi_mode();
        emulator.start_selection(
            SelectionKind::Cells,
            CellPoint { row: 0, col: 0 },
            Side::Left,
        );
        emulator.update_selection(CellPoint { row: 0, col: 2 }, Side::Right);
        let selection = emulator.term.selection.clone();
        assert!(emulator.scroll(Scroll::Lines(0)));
        assert_eq!(emulator.display_offset(), 0);
        assert_ne!(emulator.term.selection, selection);
        assert!(!emulator.scroll(Scroll::Lines(0)));
        emulator.term.vi_mode_cursor.point.line = Line(-100);
        assert!(emulator.scroll(Scroll::Bottom));
        assert_eq!(emulator.term.vi_mode_cursor.point.line, Line(0));
    }

    #[test]
    fn preparation_clears_even_an_empty_selection_once() {
        let mut emulator = Emulator::new(TermSize::default(), EmulatorOptions::default());
        assert!(!emulator.prepare_input());
        emulator.start_selection(
            SelectionKind::Cells,
            CellPoint { row: 0, col: 0 },
            Side::Left,
        );
        assert!(emulator.selection_text().is_none());
        assert!(emulator.prepare_input());
        assert!(emulator.term.selection.is_none());
        assert!(!emulator.prepare_input());
    }
}
