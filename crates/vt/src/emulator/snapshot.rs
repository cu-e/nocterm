use super::*;

impl Emulator {
    /// Writes the current viewport into `frame`.
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub fn snapshot(&self, frame: &mut Frame) {
        let content = self.term.renderable_content();
        let cols = usize::from(self.size.cols);
        let rows = usize::from(self.size.rows);
        let display_offset = content.display_offset as i32;

        frame.size = self.size;
        frame.display_offset = content.display_offset;
        frame.history_size = self.term.grid().history_size();
        frame.alt_screen = self.term.mode().contains(TermMode::ALT_SCREEN);
        let first_line = Line(-display_offset);
        frame.starts_with_continuation = first_line > self.term.grid().topmost_line()
            && self.term.grid()[first_line - 1i32][Column(cols - 1)]
                .flags
                .contains(Flags::WRAPLINE);
        frame.wraps.clear();
        frame.wraps.extend((0..rows).map(|row| {
            self.term.grid()[Line(row as i32 - display_offset)][Column(cols - 1)]
                .flags
                .contains(Flags::WRAPLINE)
        }));
        frame.lines.clear();
        frame.lines.resize(rows, None);
        if !frame.alt_screen {
            let grid = self.term.grid();
            for (row, metadata) in frame.lines.iter_mut().enumerate() {
                let line = alacritty_terminal::index::Line(row as i32 - display_offset);
                if let Some(mark) = grid[line][..].iter().find_map(|cell| cell.logical_line()) {
                    let continuation = line > grid.topmost_line()
                        && grid[line - 1i32][alacritty_terminal::index::Column(cols - 1)]
                            .flags
                            .contains(Flags::WRAPLINE);
                    *metadata = Some(LineMetadata {
                        number: mark.id,
                        timestamp_ms: mark.timestamp_ms,
                        continuation,
                    });
                }
            }
        }
        frame.combining.clear();
        frame.cells.clear();
        frame.cells.resize(cols * rows, Cell::default());

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + display_offset;
            let col = indexed.point.column.0;
            if row < 0 || row as usize >= rows || col >= cols {
                continue;
            }
            let ix = row as usize * cols + col;
            let cell = indexed.cell;
            let flags = cell.flags;

            frame.cells[ix] = Cell {
                ch: cell.c,
                fg: self.resolve(cell.fg),
                bg: self.resolve(cell.bg),
                style: Style {
                    bold: flags.contains(Flags::BOLD),
                    italic: flags.contains(Flags::ITALIC),
                    dim: flags.contains(Flags::DIM),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    strikeout: flags.contains(Flags::STRIKEOUT),
                    inverse: flags.contains(Flags::INVERSE),
                    hidden: flags.contains(Flags::HIDDEN),
                },
                wide: flags.contains(Flags::WIDE_CHAR),
                spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                selected: content
                    .selection
                    .is_some_and(|selection| selection.contains(indexed.point)),
                search_hit: self.search_match.is_some_and(|(generation, range)| {
                    if generation != self.generation {
                        return false;
                    }
                    let point = (indexed.point.line.0, col);
                    let end_cell = &self.term.grid()
                        [Point::new(Line(range.end.line), Column(usize::from(range.end.column)))];
                    let end_col = usize::from(range.end.column)
                        + usize::from(end_cell.flags.contains(Flags::WIDE_CHAR));
                    point >= (range.start.line, usize::from(range.start.column))
                        && point <= (range.end.line, end_col)
                }),
            };
            if let Some(combining) = cell.zerowidth().filter(|chars| !chars.is_empty()) {
                frame.combining.push((ix, combining.iter().collect()));
            }
        }

        let cursor_row = content.cursor.point.line.0 + display_offset;
        let cursor_col = content.cursor.point.column.0;
        let shape = match content.cursor.shape {
            AnsiCursorShape::Hidden => None,
            AnsiCursorShape::Block => Some(CursorShape::Block),
            AnsiCursorShape::Beam => Some(CursorShape::Bar),
            AnsiCursorShape::Underline => Some(CursorShape::Underline),
            AnsiCursorShape::HollowBlock => Some(CursorShape::Hollow),
        };
        frame.cursor = shape
            .filter(|_| cursor_row >= 0 && (cursor_row as usize) < rows && cursor_col < cols)
            .map(|shape| Cursor {
                row: cursor_row as u16,
                col: cursor_col as u16,
                shape,
                blinking: self.term.cursor_style().blinking,
                wide: frame.cells[cursor_row as usize * cols + cursor_col].wide,
            });
    }
}
